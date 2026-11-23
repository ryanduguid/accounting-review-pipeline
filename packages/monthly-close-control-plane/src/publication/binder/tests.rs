// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use super::*;

#[test]
#[allow(clippy::too_many_lines)]
fn write_atomic_and_collect_entries_cover_io_and_toctou_errors() {
    let dir = std::env::temp_dir().join(format!("binder-unit-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let target = dir.join("out.zip");
    assert!(
        write_atomic_with(
            Path::new("bare.zip"),
            b"x",
            &mut write_file_sync,
            &mut |a, b| fs::rename(a, b)
        )
        .unwrap_err()
        .contains("parent directory")
    );
    let bad_parent = dir.join("file_not_dir");
    fs::write(&bad_parent, b"x").unwrap();
    assert!(
        write_atomic_with(
            &bad_parent.join("out.zip"),
            b"x",
            &mut write_file_sync,
            &mut |a, b| fs::rename(a, b)
        )
        .unwrap_err()
        .contains("Cannot create binder directory")
    );
    assert!(
        write_atomic_with(
            &target,
            b"x",
            &mut |_, _| Err(io::Error::other("disk full")),
            &mut |a, b| fs::rename(a, b)
        )
        .unwrap_err()
        .contains("disk full")
    );
    assert!(
        write_atomic_with(&target, b"x", &mut write_file_sync, &mut |_, _| Err(
            io::Error::other("rename denied")
        ))
        .unwrap_err()
        .contains("rename denied")
    );
    fs::write(&target, b"existing").unwrap();
    assert!(
        write_atomic_with(&target, b"x", &mut write_file_sync, &mut |a, b| {
            fs::rename(a, b)
        })
        .unwrap_err()
        .contains("overwrite")
    );

    let verified = viewer::VerifiedPack {
        document: Json::object([]),
        summary_text: String::new(),
        exceptions: Vec::new(),
        queries: None,
        artefact_sha256: PACK_NAMES
            .into_iter()
            .map(|n| (n.to_owned(), sha256(b"orig")))
            .collect(),
    };
    let req = BinderRequest {
        pack_dir: dir.clone(),
        output_zip: target.clone(),
        attachments: Vec::new(),
        audit_chain: None,
    };
    assert!(
        collect_entries(&req, &verified, &mut |_| Err(io::Error::other("gone")))
            .err()
            .unwrap()
            .contains("Cannot read verified pack artefact")
    );
    assert!(
        collect_entries(&req, &verified, &mut |_| Ok(b"tampered".to_vec()))
            .err()
            .unwrap()
            .contains("changed during binder export")
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn zip_header_and_corruption_paths_are_rejected() {
    assert!(read_zip(b"short").unwrap_err().contains("too short"));
    let mut bad_eocd = vec![0u8; 22];
    assert!(
        read_zip(&bad_eocd)
            .unwrap_err()
            .contains("end-of-central-directory")
    );
    bad_eocd[0..4].copy_from_slice(&EOCD_SIG.to_le_bytes());
    bad_eocd[12..16].copy_from_slice(&99u32.to_le_bytes());
    assert!(read_zip(&bad_eocd).unwrap_err().contains("bounds"));

    let mut map = BTreeMap::new();
    map.insert("a.txt".to_owned(), b"hello".to_vec());
    let good = write_zip(&map);
    assert_eq!(read_zip(&good).unwrap(), map);

    let cd_off = read_u32(&good, good.len() - 6).unwrap() as usize;
    for (pos, byte, expected) in [
        (0, 0, "local file header"),
        (8, 8, "stored"),
        (30, b'b', "local header mismatch"),
        (30, 0xff, "UTF-8"),
        (35, b'X', "CRC-32"),
        (cd_off, 0, "central directory header"),
        (cd_off + 10, 8, "stored"),
        (cd_off + 20, 9, "mismatched stored sizes"),
        (cd_off + 46, 0xff, "UTF-8"),
    ] {
        let mut z = good.clone();
        z[pos] = byte;
        assert!(read_zip(&z).unwrap_err().contains(expected), "{expected}");
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn zip_truncation_duplicates_trailing_and_source_hash_errors_are_rejected() {
    let mut map = BTreeMap::new();
    map.insert("a.txt".to_owned(), b"hello".to_vec());
    let good = write_zip(&map);
    let cd_off = read_u32(&good, good.len() - 6).unwrap() as usize;
    let eocd = good.len() - 22;
    let cd_entry = good[cd_off..eocd].to_vec();

    assert!(read_u16(&[1], 0).is_err());
    assert!(read_u32(&[1, 2, 3], 0).is_err());
    assert!(
        read_local_entry(&good[..31], 0, "a.txt", 0, 5)
            .unwrap_err()
            .contains("local entry name")
    );
    assert!(
        read_local_entry(&good[..36], 0, "a.txt", crc32(b"hello"), 5)
            .unwrap_err()
            .contains("payload")
    );
    assert!(
        read_central_entry(&good[..cd_off + 47], cd_off)
            .unwrap_err()
            .contains("central directory name")
    );

    let mut dup = good.clone();
    dup.splice(eocd..eocd, cd_entry.clone());
    let new_eocd = dup.len() - 22;
    dup[new_eocd + 10..new_eocd + 12].copy_from_slice(&2u16.to_le_bytes());
    dup[new_eocd + 12..new_eocd + 16].copy_from_slice(&((cd_entry.len() * 2) as u32).to_le_bytes());
    assert!(read_zip(&dup).unwrap_err().contains("duplicate entry"));

    let mut trailing = good.clone();
    trailing.insert(eocd, 0);
    let t_eocd = trailing.len() - 22;
    trailing[t_eocd + 12..t_eocd + 16]
        .copy_from_slice(&((cd_entry.len() + 1) as u32).to_le_bytes());
    assert!(read_zip(&trailing).unwrap_err().contains("trailing bytes"));

    assert!(read_source_hashes(&Json::object([])).is_err());
    let bad_key = Json::Object(vec![(
        PyStr::from("source_sha256"),
        Json::Object(vec![(PyStr(vec![0xd800]), Json::str("abc"))]),
    )]);
    assert!(read_source_hashes(&bad_key).is_err());
    let bad_val = Json::object([("source_sha256", Json::object([("a", Json::int(1))]))]);
    assert!(read_source_hashes(&bad_val).is_err());
}

#[test]
#[allow(clippy::too_many_lines)]
fn manifest_verification_rejects_every_tampered_shape() {
    let mut base = BTreeMap::new();
    for name in PACK_NAMES {
        base.insert(name.to_owned(), b"{}".to_vec());
    }
    base.insert(
        PACK_NAMES[0].to_owned(),
        br#"{"overall_status":"PASS","source_sha256":{"current":"aaa"}}"#.to_vec(),
    );
    // Missing manifest.
    assert!(
        verify_binder_bytes(&write_zip(&base))
            .unwrap_err()
            .contains("missing manifest.json")
    );
    // Non-UTF8 manifest.
    let mut non_utf8 = base.clone();
    non_utf8.insert(MANIFEST_PATH.to_owned(), vec![0xff, 0xfe]);
    assert!(
        verify_binder_bytes(&write_zip(&non_utf8))
            .unwrap_err()
            .contains("valid UTF-8")
    );
    // Malformed JSON manifest.
    let mut bad_json = base.clone();
    bad_json.insert(MANIFEST_PATH.to_owned(), b"{".to_vec());
    assert!(
        verify_binder_bytes(&write_zip(&bad_json))
            .unwrap_err()
            .contains("valid JSON")
    );
    // Wrong schema.
    let mut bad_schema = base.clone();
    bad_schema.insert(MANIFEST_PATH.to_owned(), br#"{"schema":"other"}"#.to_vec());
    assert!(
        verify_binder_bytes(&write_zip(&bad_schema))
            .unwrap_err()
            .contains("close-binder-manifest-v1")
    );
    // Missing entries array.
    let mut no_entries = base.clone();
    no_entries.insert(
        MANIFEST_PATH.to_owned(),
        format!(r#"{{"schema":"{MANIFEST_SCHEMA}"}}"#).into_bytes(),
    );
    assert!(
        verify_binder_bytes(&write_zip(&no_entries))
            .unwrap_err()
            .contains("missing entries")
    );
    // Bad manifest_entries_sha256.
    let mut bad_entries_sha = base.clone();
    bad_entries_sha.insert(
        MANIFEST_PATH.to_owned(),
        format!(r#"{{"schema":"{MANIFEST_SCHEMA}","entries":[],"manifest_entries_sha256":"bad"}}"#)
            .into_bytes(),
    );
    assert!(
        verify_binder_bytes(&write_zip(&bad_entries_sha))
            .unwrap_err()
            .contains("manifest_entries_sha256")
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn manifest_entries_pack_and_audit_mismatch_paths_are_rejected() {
    let make_manifest =
        |entries_arr: &Json, status: &str, src_sha: &str, audit_head: Option<&str>| {
            let digest = sha256(pyjson::canonical(entries_arr).as_bytes());
            let mut m = Json::object([
                ("schema", Json::str(MANIFEST_SCHEMA)),
                ("pack_status", Json::str(status)),
                (
                    "source_sha256",
                    Json::object([("current", Json::str(src_sha))]),
                ),
                ("entries", entries_arr.clone()),
                ("manifest_entries_sha256", Json::str(&digest)),
            ]);
            if let Some(h) = audit_head {
                m.set("audit_chain_head_sha256", Json::str(h));
            }
            pyjson::pretty_utf8(&m).unwrap().into_bytes()
        };

    let src_sha = "a".repeat(64);
    let pack = crate::engine::CloseReviewPack {
        status: crate::models::Status::Pass,
        current_report_dates: vec!["2026-08-31".into()],
        prior_report_dates: vec!["2026-07-31".into()],
        source_hashes: [("current".to_owned(), src_sha.clone())]
            .into_iter()
            .collect(),
        absolute_threshold: crate::decimal::Decimal::parse("1000").unwrap(),
        percentage_threshold: crate::decimal::Decimal::parse("0.10").unwrap(),
        reconciliation_tolerance: crate::decimal::Decimal::parse("0.01").unwrap(),
        gst_tolerance: None,
        exceptions: Vec::new(),
        acknowledgement: None,
        calculation_evidence: Vec::new(),
        required_calculations: Vec::new(),
        required_controls: Vec::new(),
        controls_not_run: Vec::new(),
        relied_on: BTreeSet::new(),
        equity_reconciliation: None,
        producer: None,
    };
    let rendered = crate::report::rendered(&pack).unwrap();
    let mut files = BTreeMap::new();
    for (name, data) in PACK_NAMES.into_iter().zip(rendered) {
        files.insert(name.to_owned(), data);
    }
    let entry_obj = |p: &str, data: &[u8]| {
        Json::object([
            ("path", Json::str(p)),
            ("bytes", Json::int(data.len() as i64)),
            ("sha256", Json::str(&sha256(data))),
        ])
    };

    let bad_len_arr = Json::Array(vec![Json::object([
        ("path", Json::str(PACK_NAMES[0])),
        ("bytes", Json::str("nope")),
        ("sha256", Json::str(&src_sha)),
    ])]);
    let mut z = files.clone();
    z.insert(
        MANIFEST_PATH.to_owned(),
        make_manifest(&bad_len_arr, "PASS", &src_sha, None),
    );
    assert!(
        verify_binder_bytes(&write_zip(&z))
            .unwrap_err()
            .contains("invalid byte length")
    );

    let missing_arr = Json::Array(vec![entry_obj("ghost.txt", b"hi")]);
    let mut z = files.clone();
    z.insert(
        MANIFEST_PATH.to_owned(),
        make_manifest(&missing_arr, "PASS", &src_sha, None),
    );
    assert!(
        verify_binder_bytes(&write_zip(&z))
            .unwrap_err()
            .contains("missing manifest entry")
    );

    let wrong_sha_arr = Json::Array(vec![Json::object([
        ("path", Json::str(PACK_NAMES[1])),
        (
            "bytes",
            Json::int(files.get(PACK_NAMES[1]).unwrap().len() as i64),
        ),
        ("sha256", Json::str("0000")),
    ])]);
    let mut z = files.clone();
    z.insert(
        MANIFEST_PATH.to_owned(),
        make_manifest(&wrong_sha_arr, "PASS", &src_sha, None),
    );
    assert!(
        verify_binder_bytes(&write_zip(&z))
            .unwrap_err()
            .contains("failed digest")
    );

    let dup_arr = Json::Array(vec![
        entry_obj(PACK_NAMES[1], files.get(PACK_NAMES[1]).unwrap()),
        entry_obj(PACK_NAMES[1], files.get(PACK_NAMES[1]).unwrap()),
    ]);
    let mut z = files.clone();
    z.insert(
        MANIFEST_PATH.to_owned(),
        make_manifest(&dup_arr, "PASS", &src_sha, None),
    );
    assert!(
        verify_binder_bytes(&write_zip(&z))
            .unwrap_err()
            .contains("duplicate entry")
    );

    let valid_pack_entries: Vec<Json> = PACK_NAMES
        .iter()
        .map(|n| entry_obj(n, files.get(*n).unwrap()))
        .collect();
    let mut z = files.clone();
    z.insert("sneaky.txt".to_owned(), b"extra".to_vec());
    z.insert(
        MANIFEST_PATH.to_owned(),
        make_manifest(
            &Json::Array(valid_pack_entries.clone()),
            "PASS",
            &src_sha,
            None,
        ),
    );
    assert!(
        verify_binder_bytes(&write_zip(&z))
            .unwrap_err()
            .contains("not listed")
    );

    let mut partial_files = BTreeMap::new();
    partial_files.insert("extra.txt".to_owned(), b"hi".to_vec());
    partial_files.insert(
        MANIFEST_PATH.to_owned(),
        make_manifest(
            &Json::Array(vec![entry_obj("extra.txt", b"hi")]),
            "PASS",
            &src_sha,
            None,
        ),
    );
    assert!(
        verify_binder_bytes(&write_zip(&partial_files))
            .unwrap_err()
            .contains("missing required pack artefact")
    );

    let mut z = files.clone();
    z.insert(PACK_NAMES[0].to_owned(), b"{".to_vec());
    let arr = Json::Array(
        PACK_NAMES
            .iter()
            .map(|n| entry_obj(n, z.get(*n).unwrap()))
            .collect(),
    );
    z.insert(
        MANIFEST_PATH.to_owned(),
        make_manifest(&arr, "PASS", &src_sha, None),
    );
    assert!(verify_binder_bytes(&write_zip(&z)).is_err());

    let mut z = files.clone();
    z.insert(
        MANIFEST_PATH.to_owned(),
        make_manifest(
            &Json::Array(valid_pack_entries.clone()),
            "REVIEW",
            &src_sha,
            None,
        ),
    );
    assert!(
        verify_binder_bytes(&write_zip(&z))
            .unwrap_err()
            .contains("pack_status")
    );

    let mut z = files.clone();
    z.insert(
        MANIFEST_PATH.to_owned(),
        make_manifest(
            &Json::Array(valid_pack_entries.clone()),
            "PASS",
            "bbb",
            None,
        ),
    );
    assert!(
        verify_binder_bytes(&write_zip(&z))
            .unwrap_err()
            .contains("source_sha256")
    );

    let mut z = files.clone();
    z.insert(
        MANIFEST_PATH.to_owned(),
        make_manifest(
            &Json::Array(valid_pack_entries.clone()),
            "PASS",
            &src_sha,
            Some("deadbeef"),
        ),
    );
    assert!(
        verify_binder_bytes(&write_zip(&z))
            .unwrap_err()
            .contains("presence does not match")
    );

    let chain = build_audit_chain(&[AuditEventInput {
        timestamp: "2026-08-31T10:00:00Z".into(),
        actor: "RD".into(),
        action: "review".into(),
        target: "pack".into(),
        detail: "checked".into(),
    }])
    .unwrap();
    let mut z = files.clone();
    z.insert(AUDIT_CHAIN_PATH.to_owned(), chain.clone());
    let mut with_chain = valid_pack_entries;
    with_chain.push(entry_obj(AUDIT_CHAIN_PATH, &chain));
    z.insert(
        MANIFEST_PATH.to_owned(),
        make_manifest(
            &Json::Array(with_chain.clone()),
            "PASS",
            &src_sha,
            Some("wronghead"),
        ),
    );
    assert!(
        verify_binder_bytes(&write_zip(&z))
            .unwrap_err()
            .contains("audit_chain_head_sha256")
    );

    // Non-string audit_chain_head_sha256 triggers json_str + transpose error path.
    let bad_head_manifest = Json::object([
        ("schema", Json::str(MANIFEST_SCHEMA)),
        ("pack_status", Json::str("PASS")),
        (
            "source_sha256",
            Json::object([("current", Json::str(&src_sha))]),
        ),
        ("entries", Json::Array(with_chain.clone())),
        (
            "manifest_entries_sha256",
            Json::str(&sha256(
                pyjson::canonical(&Json::Array(with_chain)).as_bytes(),
            )),
        ),
        ("audit_chain_head_sha256", Json::int(42)),
    ]);
    let mut z2 = files;
    z2.insert(AUDIT_CHAIN_PATH.to_owned(), chain);
    z2.insert(
        MANIFEST_PATH.to_owned(),
        pyjson::pretty_utf8(&bad_head_manifest)
            .unwrap()
            .into_bytes(),
    );
    assert!(
        verify_binder_bytes(&write_zip(&z2))
            .unwrap_err()
            .contains("must be UTF-8 text")
    );
}
