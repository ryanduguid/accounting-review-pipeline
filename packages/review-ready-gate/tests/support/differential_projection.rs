use super::*;

fn review_result(example: &str, profile: &str) -> Result<Json> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(example);
    let request = Json::object([
        ("op", Json::str("review_pack")),
        ("pack", Json::str(&directory.to_string_lossy())),
        ("profile", Json::str(profile)),
        ("tolerance", Json::str("0.01")),
    ]);
    let result = probe::decode(&probe::evaluate(&probe::encode(&request))?)?;
    assert!(result.get("ok").is_some(), "{result:?}");
    Ok(result)
}

fn files(result: &Json) -> &[Json] {
    let Json::Array(files) = result.at("ok").at("files") else {
        panic!("native result must have rendered files")
    };
    files
}

fn change_file(
    result: &mut Json,
    index: usize,
    change: impl FnOnce(String) -> String,
) -> Result<()> {
    let mut changed = files(result).to_vec();
    let original = String::from_utf8(unhex(&text(&changed[index]).unwrap())?)?;
    changed[index] = Json::str(&hex(change(original).as_bytes()));
    set(result, &["ok", "files"], Json::Array(changed));
    Ok(())
}

fn control(document: &Json, slot: &str) -> Json {
    let Json::Array(controls) = document.at("controls_not_run") else {
        panic!("fixture must list controls")
    };
    controls
        .iter()
        .find(|entry| entry.at("slot").is_str(slot))
        .unwrap()
        .clone()
}

fn document_without(document: &Json, slot: &str) -> Json {
    let mut expected = document.clone();
    let Json::Array(controls) = document.at("controls_not_run") else {
        panic!("fixture must list controls")
    };
    set(
        &mut expected,
        &["controls_not_run"],
        Json::Array(
            controls
                .iter()
                .filter(|entry| !entry.at("slot").is_str(slot))
                .cloned()
                .collect(),
        ),
    );
    expected
}

fn rendered_without(document: &Json, slot: &str) -> Result<Json> {
    let mut expected = document_without(document, slot);
    let profile = text(document.at("engagement_type")).unwrap();
    let slots = review_ready_gate::profiles::slots_for(&profile)?;
    let Json::Object(sources) = field_mut(&mut expected, "source_sha256").unwrap() else {
        panic!("fixture must carry source evidence")
    };
    // The probe's canonical JSON sorts keys; the renderer lists evidence in
    // profile capture order, which survives independently of the JSON order.
    sources.sort_by_key(|(key, _)| slots.iter().position(|slot| key.is(slot.name)).unwrap());
    Ok(Json::Array(
        probe::render_pack(&expected)?
            .iter()
            .map(|bytes| Json::str(&hex(bytes)))
            .collect(),
    ))
}

fn assert_refused(result: &Json, slot: &str) -> Result<()> {
    let before = probe::encode(result);
    assert!(without_absent_control(&before, slot)?.is_none());
    assert_eq!(probe::encode(result), before);
    Ok(())
}

#[test]
fn mutable_fields_require_an_object_and_preserve_siblings() -> Result<()> {
    for mut value in [
        Json::Null,
        Json::Bool(false),
        Json::Array(Vec::new()),
        Json::str("object"),
    ] {
        let before = probe::encode(&value);
        assert!(field_mut(&mut value, "slot").is_none());
        assert_eq!(probe::encode(&value), before);
    }
    let mut value = probe::decode(r#"{"target":1,"sibling":{"value":2}}"#)?;
    assert!(field_mut(&mut value, "missing").is_none());
    *field_mut(&mut value, "target").unwrap() = Json::str("changed");
    assert_eq!(
        probe::encode(&value),
        r#"{"sibling":{"value":2},"target":"changed"}"#
    );
    Ok(())
}

#[test]
fn hex_decoding_rejects_malformed_input_and_preserves_every_byte() -> Result<()> {
    assert_eq!(unhex("0").unwrap_err().to_string(), "odd pack file hex");
    for invalid in ["gg", "0z", "é0", "0é0"] {
        assert!(unhex(invalid).is_err(), "{invalid:?}");
    }
    let bytes: Vec<u8> = (0..=255).collect();
    assert_eq!(unhex(&hex(&bytes))?, bytes);
    assert_eq!(unhex("00aBFF")?, vec![0, 171, 255]);
    assert!(unhex("")?.is_empty());
    Ok(())
}

#[test]
fn control_projection_requires_exactly_one_matching_entry() -> Result<()> {
    let result = review_result("year-end-ready", "year_end")?;
    let document = result.at("ok").at("pack");
    let original = probe::encode(document);
    let selected = control(document, "fs_mapping");
    let retained = control(document, "prior_findings");
    let (projected, entry, left) = without_control_entry(document, "fs_mapping").unwrap();
    assert_eq!(left, 1);
    assert_eq!(probe::encode(&entry), probe::encode(&selected));
    let mut expected = document.clone();
    set(
        &mut expected,
        &["controls_not_run"],
        Json::Array(vec![retained]),
    );
    assert_eq!(probe::encode(&projected), probe::encode(&expected));
    assert_eq!(probe::encode(document), original);
    assert!(without_control_entry(document, "unknown").is_none());
    for controls in [
        Json::Null,
        Json::Array(Vec::new()),
        Json::Array(vec![selected.clone(), selected]),
    ] {
        let mut malformed = document.clone();
        set(&mut malformed, &["controls_not_run"], controls);
        assert!(without_control_entry(&malformed, "fs_mapping").is_none());
    }
    let mut missing = document.clone();
    remove(&mut missing, "controls_not_run");
    assert!(without_control_entry(&missing, "fs_mapping").is_none());
    Ok(())
}

#[test]
fn projected_files_match_an_independently_rendered_pack() -> Result<()> {
    for (example, profile, slot, left) in [
        ("year-end-ready", "year_end", "fs_mapping", 1),
        ("month-end-ready", "month_end", "prior_findings", 0),
    ] {
        let result = review_result(example, profile)?;
        let document = result.at("ok").at("pack");
        let expected = rendered_without(document, slot)?;
        let projected =
            without_control_files(files(&result), &control(document, slot), left, slot)?.unwrap();
        assert_eq!(probe::encode(&projected), probe::encode(&expected));
        let Json::Array(projected) = projected else {
            panic!("files")
        };
        assert_eq!(text(&projected[2]), text(&files(&result)[2]));
        if left == 0 {
            let summary = String::from_utf8(unhex(&text(&projected[1]).unwrap())?)?;
            assert!(summary.contains("Every control this engagement profile configures ran."));
        }
    }
    Ok(())
}

#[test]
fn file_projection_preserves_lf_and_crlf_rendering() -> Result<()> {
    let source = review_result("year-end-ready", "year_end")?;
    let document = source.at("ok").at("pack");
    for newline in ["\n", "\r\n"] {
        let mut result = source.clone();
        for index in 0..2 {
            change_file(&mut result, index, |text| {
                text.replace("\r\n", "\n").replace('\n', newline)
            })?;
        }
        let projected = without_control_files(
            files(&result),
            &control(document, "fs_mapping"),
            1,
            "fs_mapping",
        )?
        .unwrap();
        let mut expected = Json::object([(
            "ok",
            Json::object([("files", rendered_without(document, "fs_mapping")?)]),
        )]);
        for index in 0..2 {
            change_file(&mut expected, index, |text| {
                text.replace("\r\n", "\n").replace('\n', newline)
            })?;
        }
        assert_eq!(
            probe::encode(&projected),
            probe::encode(expected.at("ok").at("files"))
        );
    }
    Ok(())
}

#[test]
fn file_projection_refuses_missing_and_non_string_files_or_entry_fields() -> Result<()> {
    let result = review_result("year-end-ready", "year_end")?;
    let entry = control(result.at("ok").at("pack"), "fs_mapping");
    for length in 0..3 {
        assert!(
            without_control_files(&files(&result)[..length], &entry, 1, "fs_mapping")?.is_none()
        );
    }
    for index in 0..3 {
        let mut malformed = files(&result).to_vec();
        malformed[index] = Json::Null;
        assert!(without_control_files(&malformed, &entry, 1, "fs_mapping")?.is_none());
    }
    for key in ["filename", "reason"] {
        let mut malformed = entry.clone();
        set(&mut malformed, &[key], Json::Bool(false));
        assert!(without_control_files(files(&result), &malformed, 1, "fs_mapping")?.is_none());
    }
    Ok(())
}

#[test]
fn file_projection_refuses_changed_or_duplicated_markers_and_counts() -> Result<()> {
    let source = review_result("year-end-ready", "year_end")?;
    let entry = control(source.at("ok").at("pack"), "fs_mapping");
    for index in 0..2 {
        for duplicate in [false, true] {
            let mut result = source.clone();
            change_file(&mut result, index, |text| {
                if duplicate {
                    text.repeat(2)
                } else {
                    text.replace("fs_mapping", "changed")
                }
            })?;
            assert!(without_control_files(files(&result), &entry, 1, "fs_mapping")?.is_none());
        }
    }
    let summary = String::from_utf8(unhex(&text(&files(&source)[1]).unwrap())?)?;
    let newline = if summary.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    for replacement in [
        "- Controls not run: 7.".to_owned(),
        format!("- Controls not run: 2.{newline}- Controls not run: 2."),
    ] {
        let mut result = source.clone();
        change_file(&mut result, 1, |text| {
            text.replace("- Controls not run: 2.", &replacement)
        })?;
        assert!(without_control_files(files(&result), &entry, 1, "fs_mapping")?.is_none());
    }
    Ok(())
}

#[test]
fn stdout_projection_preserves_other_lines_and_line_endings() {
    for newline in ["\n", "\r\n", ""] {
        let prefix = "READY\nreview-ready: preserved diagnostic\n";
        let original = format!(
            "{prefix}review-ready: 2 control(s) did not run: fs_mapping, prior_findings{newline}"
        );
        let expected =
            format!("{prefix}review-ready: 1 control(s) did not run: prior_findings{newline}");
        assert_eq!(
            without_control_line(&original, "fs_mapping"),
            Some(expected)
        );
        let original =
            format!("{prefix}review-ready: 1 control(s) did not run: fs_mapping{newline}");
        assert_eq!(
            without_control_line(&original, "fs_mapping"),
            Some(prefix.into())
        );
    }
}

#[test]
fn stdout_projection_refuses_wrong_counts_and_ambiguous_targets() {
    let valid = "review-ready: 1 control(s) did not run: fs_mapping\n";
    for stdout in [
        String::new(),
        "READY\n".into(),
        valid.repeat(2),
        "review-ready: 2 control(s) did not run: fs_mapping\n".into(),
        "review-ready: 2 control(s) did not run: fs_mapping, fs_mapping\n".into(),
        "review-ready: 1 control(s) did not run: prior_findings\n".into(),
        "review-ready: x control(s) did not run: fs_mapping\n".into(),
    ] {
        assert!(
            without_control_line(&stdout, "fs_mapping").is_none(),
            "{stdout:?}"
        );
    }
}

#[test]
fn result_projection_requires_its_native_envelope_and_readable_first_file() -> Result<()> {
    let source = review_result("year-end-ready", "year_end")?;
    for malformed in [
        Json::Null,
        Json::object([]),
        Json::object([("ok", Json::Null)]),
    ] {
        assert_refused(&malformed, "fs_mapping")?;
    }
    let mut missing = source.clone();
    remove(field_mut(&mut missing, "ok").unwrap(), "files");
    assert_refused(&missing, "fs_mapping")?;
    for malformed_files in [
        Json::Null,
        Json::Array(Vec::new()),
        Json::Array(vec![Json::Null]),
    ] {
        let mut malformed = source.clone();
        set(&mut malformed, &["ok", "files"], malformed_files);
        assert_refused(&malformed, "fs_mapping")?;
    }
    Ok(())
}

#[test]
fn result_projection_refuses_inconsistent_pack_summary_or_stdout() -> Result<()> {
    let source = review_result("year-end-ready", "year_end")?;
    let mut inconsistent = source.clone();
    set(
        &mut inconsistent,
        &["ok", "pack", "status"],
        Json::str("BLOCKED"),
    );
    assert_refused(&inconsistent, "fs_mapping")?;
    let mut summary = source.clone();
    change_file(&mut summary, 1, |text| {
        text.replace("fs_mapping", "unexpected")
    })?;
    assert_refused(&summary, "fs_mapping")?;
    for stdout in [Json::Null, Json::str("READY\n")] {
        let mut malformed = source.clone();
        set(&mut malformed, &["ok", "stdout"], stdout);
        assert_refused(&malformed, "fs_mapping")?;
    }
    Ok(())
}

#[test]
fn complete_result_projection_preserves_the_envelope_and_rendered_bytes() -> Result<()> {
    let source = review_result("year-end-ready", "year_end")?;
    let document = source.at("ok").at("pack");
    let mut expected = source.clone();
    set(
        &mut expected,
        &["ok", "pack"],
        document_without(document, "fs_mapping"),
    );
    set(
        &mut expected,
        &["ok", "files"],
        rendered_without(document, "fs_mapping")?,
    );
    let actual = without_absent_control(&probe::encode(&source), "fs_mapping")?.unwrap();
    assert_eq!(actual, probe::encode(&expected));
    let mut files_only = source.clone();
    remove(field_mut(&mut files_only, "ok").unwrap(), "pack");
    remove(field_mut(&mut expected, "ok").unwrap(), "pack");
    assert_eq!(
        without_absent_control(&probe::encode(&files_only), "fs_mapping")?.unwrap(),
        probe::encode(&expected)
    );
    Ok(())
}

#[test]
fn malformed_encoded_files_return_errors_instead_of_a_projection() -> Result<()> {
    let source = review_result("year-end-ready", "year_end")?;
    for encoded in ["0", "gg", "ff", "7b"] {
        let mut malformed = source.clone();
        let mut changed = files(&source).to_vec();
        changed[0] = Json::str(encoded);
        set(&mut malformed, &["ok", "files"], Json::Array(changed));
        assert!(without_absent_control(&probe::encode(&malformed), "fs_mapping").is_err());
    }
    Ok(())
}

#[test]
fn genuine_cli_result_projects_files_and_diagnostic_together() -> Result<()> {
    let scratch = Scratch::new()?;
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/year-end-ready");
    let output = scratch.0.join("cli");
    let args = [
        "gate",
        "--profile",
        "year_end",
        "--pack",
        &directory.to_string_lossy(),
        "--output",
        &output.to_string_lossy(),
    ];
    let request = Json::object([
        ("op", Json::str("cli")),
        (
            "args",
            Json::Array(args.iter().map(|arg| Json::str(arg)).collect()),
        ),
        ("output_files", Json::str(&output.to_string_lossy())),
    ]);
    let source = probe::decode(&probe::evaluate(&probe::encode(&request))?)?;
    let document = probe::decode(&String::from_utf8(unhex(
        &text(&files(&source)[0]).unwrap(),
    )?)?)?;
    let stdout = text(source.at("ok").at("stdout")).unwrap();
    let line = "review-ready: 2 control(s) did not run: fs_mapping, prior_findings";
    assert_eq!(stdout.matches(line).count(), 1);
    let mut expected = source.clone();
    set(
        &mut expected,
        &["ok", "stdout"],
        Json::str(&stdout.replace(
            line,
            "review-ready: 1 control(s) did not run: prior_findings",
        )),
    );
    set(
        &mut expected,
        &["ok", "files"],
        rendered_without(&document, "fs_mapping")?,
    );
    assert_eq!(
        without_absent_control(&probe::encode(&source), "fs_mapping")?.unwrap(),
        probe::encode(&expected)
    );
    Ok(())
}
