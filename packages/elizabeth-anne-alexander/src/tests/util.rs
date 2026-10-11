//! Port of tests/test_util.py.

use super::*;
use crate::util::{Location, load_json_exact, load_json_object};

fn artefact(dir: &TempDir, content: &[u8]) -> Location {
    let path = dir.path().join("artefact.json");
    std::fs::write(&path, content).unwrap();
    Location::Disk(path)
}

#[test]
fn test_json_that_is_not_an_object_is_blocked_not_a_traceback() {
    for payload in ["[]", "\"text\"", "3"] {
        let dir = TempDir::new();
        let env = env(dir.path());
        let location = artefact(&dir, payload.as_bytes());
        assert_refused(
            load_json_object(&env, &location, "test artefact"),
            "must be a JSON object",
        );
        assert_refused(
            load_json_exact(&env, &location, &[], "test artefact"),
            "must be a JSON object",
        );
    }
}

#[test]
fn test_a_key_set_that_is_not_exactly_the_declared_one_is_refused() {
    for payload in [
        r#"{"schema_version": "v1", "run_id": "sha256:0", "mode": "live"}"#,
        r#"{"schema_version": "v1"}"#,
    ] {
        let dir = TempDir::new();
        let env = env(dir.path());
        let result = load_json_exact(
            &env,
            &artefact(&dir, payload.as_bytes()),
            &["schema_version", "run_id"],
            "test artefact",
        );
        assert_refused(result, "test artefact must contain exactly: run_id, schema_version");
    }
}

#[test]
fn test_the_exact_declared_key_set_is_accepted() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let loaded = load_json_exact(
        &env,
        &artefact(&dir, br#"{"schema_version": "v1", "run_id": "sha256:0"}"#),
        &["schema_version", "run_id"],
        "test artefact",
    );
    assert_eq!(
        pyjson::canonical(&loaded.unwrap()),
        r#"{"run_id":"sha256:0","schema_version":"v1"}"#
    );
}

#[test]
fn test_a_duplicate_key_at_any_depth_is_refused_not_last_key_wins() {
    for (payload, duplicated) in [
        (
            r#"{"schema_version": "v1", "schema_version": "v2", "run_id": "sha256:0"}"#,
            "schema_version",
        ),
        (
            r#"{"schema_version": "v1", "run_id": {"sha256": "a", "sha256": "b"}}"#,
            "sha256",
        ),
    ] {
        let dir = TempDir::new();
        let env = env(dir.path());
        let location = artefact(&dir, payload.as_bytes());
        let expected = format!("test artefact has duplicate key '{duplicated}'");
        assert_refused(load_json_object(&env, &location, "test artefact"), &expected);
        assert_refused(
            load_json_exact(&env, &location, &["schema_version", "run_id"], "test artefact"),
            &expected,
        );
    }
}

#[test]
fn test_non_utf8_json_is_blocked_not_a_traceback() {
    let dir = TempDir::new();
    let env = env(dir.path());
    assert_refused(
        load_json_exact(&env, &artefact(&dir, b"\xff\xfe{}"), &[], "test artefact"),
        "not valid UTF-8",
    );
}

#[test]
fn test_unreadable_path_is_blocked_not_a_traceback() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let result = load_json_exact(&env, &Location::Disk(dir.path().to_path_buf()), &[], "test artefact");
    assert_refused(result, "cannot be read");
}

#[cfg(unix)]
#[test]
fn test_a_name_the_operating_system_refuses_is_blocked_not_a_traceback() {
    use std::os::unix::ffi::OsStrExt;
    // An embedded NUL is the name every platform refuses outright.
    let dir = TempDir::new();
    let env = env(dir.path());
    let target = std::path::Path::new(std::ffi::OsStr::from_bytes(b"build/de\0mo/decision.json"));
    assert_refused(env.within_build(target, "test artefact", true), "not a usable path");
}

#[test]
fn test_a_name_that_cannot_be_encoded_for_the_filesystem_is_blocked_not_a_traceback() {
    // POSIX resolve refuses a lone surrogate with UnicodeEncodeError; Windows
    // passes it to the filesystem, which reports a missing file.
    let dir = TempDir::new();
    let env = env(dir.path());
    let name = crate::pyjson::PyStr(vec!['a' as u32, 0xD800, 'b' as u32]);
    let expected = if cfg!(windows) {
        "does not exist"
    } else {
        "not a usable path"
    };
    assert_refused(env.within_bundle(&name, "samples", "x"), expected);
}

#[cfg(not(windows))]
#[test]
fn test_bundled_paths_through_missing_or_file_parts_are_refused_as_posix_resolve_does() {
    // POSIX strict resolve visits every part, so a later ".." cannot excuse a
    // missing directory or a file used as one. Windows normalises ".." first.
    let dir = TempDir::new();
    let env = env(dir.path());
    let missing = PyStr::from("samples/nonexistent/../contexts/sample-monthly-variance.context.json");
    assert_refused(
        env.within_bundle(&missing, "samples", "context"),
        "context does not exist",
    );
    let through_file =
        PyStr::from("samples/contexts/sample-monthly-variance.context.json/../sample-monthly-variance.context.json");
    assert_refused(
        env.within_bundle(&through_file, "samples", "context"),
        "context is not a usable path",
    );
    let back_and_forth = PyStr::from("samples/contexts/../contexts/sample-monthly-variance.context.json");
    assert!(env.within_bundle(&back_and_forth, "samples", "context").is_ok());
}
