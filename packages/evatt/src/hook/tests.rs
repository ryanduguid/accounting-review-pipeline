// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use super::*;
use std::io;
use tempfile::TempDir;

struct FailingReader;
impl io::Read for FailingReader {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("synthetic stdin failure"))
    }
}

struct FailingWriter;
impl io::Write for FailingWriter {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("synthetic stdout failure"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn failure_mode_1_raw_string_keying_assigns_one_token_across_spellings() {
    let mut map = SessionMap::default();
    let first = mask_tool_result(
        r#"{"client_tfn":"123456782","abn":"51 824 753 556","medicare":"2123 45670 1/1"}"#,
        &mut map,
    )
    .unwrap();
    let second = mask_tool_result(
        r#"{"client_tfn":"123 456 782","abn":"51824753556","medicare":"2123456701"}"#,
        &mut map,
    )
    .unwrap();
    assert_eq!(
        first,
        r#"{"client_tfn":"TFN_01","abn":"ABN_01","medicare":"MEDICARE_01"}"#
    );
    assert_eq!(second, first);
    assert_eq!(map.entries.len(), 3);
    assert_eq!(
        restore_tool_arguments(r#"{"tfn":"TFN_01","abn":"ABN_01"}"#, &map).unwrap(),
        RestoreOutcome::Restored(r#"{"tfn":"123456782","abn":"51 824 753 556"}"#.into())
    );
}

#[test]
fn failure_mode_2_unmapped_placeholder_refuses_with_exit_2_and_reread_guidance() {
    let tmp = TempDir::new().unwrap();
    let session = tmp.path().join("hook-session.json");
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run_hook_io(
        "post-tool",
        &session,
        None,
        None,
        "TFN 123456782".as_bytes(),
        &mut out,
        &mut err,
    );
    assert_eq!(code, 0);
    assert_eq!(String::from_utf8(out).unwrap(), "TFN TFN_01");
    assert!(err.is_empty());

    let mut pre_out = Vec::new();
    let mut pre_err = Vec::new();
    let code = run_hook_io(
        "pre-tool",
        &session,
        None,
        None,
        r#"{"tfn":"TFN_01","other":"TFN_99","abn":"ABN_02"}"#.as_bytes(),
        &mut pre_out,
        &mut pre_err,
    );
    assert_eq!(code, 2);
    assert!(pre_out.is_empty());
    let msg = String::from_utf8(pre_err).unwrap();
    assert!(msg.contains("ABN_02") && msg.contains("TFN_99") && msg.contains("re-read the source"));
}

#[test]
fn failure_mode_3_paths_file_extensions_and_scanner_stamps_are_exempt_while_real_ids_mask() {
    let mut map = SessionMap::default();
    let input = concat!(
        r#"{"win_path":"C:\clients\123456782\return.pdf","#,
        r#""unix_path":"/work/51824753556/tb.csv","#,
        r#""file":"123456782.pdf","#,
        r#""scan":"Scan_20260630.pdf","#,
        r#""scan_tfn":"Scan_123456782.pdf","#,
        r#""invalid_tfn":"TFN: 123456789","#,
        r#""email":"demo@example.invalid","#,
        r#""real_tfn":"123456782."}"#,
    );
    let masked = mask_tool_result(input, &mut map).unwrap();
    assert!(masked.contains(r#"C:\clients\123456782\return.pdf"#), "{masked}");
    assert!(masked.contains(r#"/work/51824753556/tb.csv"#), "{masked}");
    assert!(masked.contains(r#""file":"123456782.pdf""#), "{masked}");
    assert!(masked.contains(r#""scan":"Scan_20260630.pdf""#), "{masked}");
    assert!(masked.contains(r#""scan_tfn":"Scan_123456782.pdf""#), "{masked}");
    assert!(masked.contains(r#""invalid_tfn":"TFN: 123456789""#), "{masked}");
    assert!(masked.contains(r#""email":"demo@example.invalid""#), "{masked}");
    assert!(masked.contains(r#""real_tfn":"TFN_01.""#), "{masked}");
    assert_eq!(map.entries.len(), 1);
}

fn vec_args(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn hook_file_io_round_trip_and_missing_session_error() {
    let tmp = TempDir::new().unwrap();
    let session = tmp.path().join("session.json");
    let input = tmp.path().join("in.json");
    let masked_path = tmp.path().join("masked.json");
    let restored_path = tmp.path().join("restored.json");
    fs::write(&input, "ACN 004 085 616\n").unwrap();

    let mut err = Vec::new();
    let code = run_hook_io(
        "pre-tool",
        &session,
        None,
        None,
        "ACN_01".as_bytes(),
        &mut Vec::new(),
        &mut err,
    );
    assert_eq!(code, 1);
    assert!(String::from_utf8(err).unwrap().contains("does not exist"));

    let mask_args = vec_args(&[
        "hook",
        "post-tool",
        "--session",
        session.to_str().unwrap(),
        "--in",
        input.to_str().unwrap(),
        "--out",
        masked_path.to_str().unwrap(),
    ]);
    assert_eq!(crate::cli::main(&mask_args, &mut Vec::new(), &mut Vec::new()), 0);
    assert_eq!(fs::read_to_string(&masked_path).unwrap(), "ACN ACN_01\n");

    let restore_args = vec_args(&[
        "hook",
        "pre-tool",
        "--session",
        session.to_str().unwrap(),
        "--in",
        masked_path.to_str().unwrap(),
        "--out",
        restored_path.to_str().unwrap(),
    ]);
    assert_eq!(crate::cli::main(&restore_args, &mut Vec::new(), &mut Vec::new()), 0);
    assert_eq!(fs::read_to_string(&restored_path).unwrap(), "ACN 004 085 616\n");
}

#[test]
fn hook_session_map_persists_ordinals_across_invocations() {
    let tmp = TempDir::new().unwrap();
    let session = tmp.path().join("session.json");
    let mut out1 = Vec::new();
    assert_eq!(
        run_hook_io(
            "post-tool",
            &session,
            None,
            None,
            "ACN 004 085 616".as_bytes(),
            &mut out1,
            &mut Vec::new(),
        ),
        0
    );
    assert_eq!(String::from_utf8(out1).unwrap(), "ACN ACN_01");

    let mut out2 = Vec::new();
    assert_eq!(
        run_hook_io(
            "post-tool",
            &session,
            None,
            None,
            "ACN 005 749 986".as_bytes(),
            &mut out2,
            &mut Vec::new(),
        ),
        0
    );
    assert_eq!(String::from_utf8(out2).unwrap(), "ACN ACN_02");
}

#[test]
fn hook_cli_help_and_mode_validation_errors() {
    let tmp = TempDir::new().unwrap();
    let session = tmp.path().join("session.json");
    let mut help_out = Vec::new();
    assert_eq!(
        crate::cli::main(&vec_args(&["hook", "--help"]), &mut help_out, &mut Vec::new()),
        0
    );
    assert!(String::from_utf8(help_out).unwrap().contains("{post-tool,pre-tool}"));
    let mut bad_err = Vec::new();
    assert_eq!(
        crate::cli::main(
            &vec_args(&["hook", "--session", session.to_str().unwrap()]),
            &mut Vec::new(),
            &mut bad_err
        ),
        1
    );
    assert!(String::from_utf8(bad_err).unwrap().contains("required: mode"));
    let mut bad_err = Vec::new();
    assert_eq!(
        crate::cli::main(
            &vec_args(&["hook", "bogus", "--session", session.to_str().unwrap()]),
            &mut Vec::new(),
            &mut bad_err,
        ),
        1
    );
    assert!(String::from_utf8(bad_err).unwrap().contains("invalid choice"));
    let mut err = Vec::new();
    assert_eq!(
        run_hook_io("other", &session, None, None, "".as_bytes(), &mut Vec::new(), &mut err),
        1
    );
    assert!(String::from_utf8(err).unwrap().contains("unknown hook mode"));
}

#[test]
fn hook_io_rejects_overwrite_and_stream_failures() {
    let tmp = TempDir::new().unwrap();
    let session = tmp.path().join("session.json");
    let mut err = Vec::new();
    let code = run_hook_io(
        "post-tool",
        &session,
        None,
        Some(&session),
        "".as_bytes(),
        &mut Vec::new(),
        &mut err,
    );
    assert_eq!(code, 1);
    assert!(String::from_utf8(err).unwrap().contains("overwrite"));
    assert_eq!(
        run_hook_io(
            "post-tool",
            &session,
            None,
            None,
            FailingReader,
            &mut Vec::new(),
            &mut Vec::new()
        ),
        1
    );
    let mut writer = FailingWriter;
    assert!(writer.flush().is_ok());
    assert_eq!(
        run_hook_io(
            "post-tool",
            &session,
            None,
            None,
            "ok".as_bytes(),
            &mut writer,
            &mut Vec::new()
        ),
        1
    );
}

#[test]
fn hook_rejects_malformed_and_directory_session_maps() {
    let tmp = TempDir::new().unwrap();
    assert!(check_owner_only(&tmp.path().join("absent-session.json")).is_err());
    let dir_session = tmp.path().join("dir-session");
    fs::create_dir(&dir_session).unwrap();
    assert!(load_session_map(&dir_session, true).is_err());
    assert!(save_session_map(&dir_session, &SessionMap::default()).is_err());

    let bad_session = tmp.path().join("bad-session.json");
    for payload in [
        "not json",
        r#"{"schema_version":2,"entries":[]}"#,
        r#"{"schema_version":1,"entries":"bad"}"#,
        r#"{"schema_version":1,"entries":[{}]}"#,
        r#"{"schema_version":1,"entries":[{"kind":"email","normalised":"a","placeholder":"TFN_01","value":"a"}]}"#,
    ] {
        write_restricted(&bad_session, payload);
        assert!(load_session_map(&bad_session, false).is_err(), "{payload}");
    }
}

fn write_restricted(path: &Path, content: &str) {
    let _ = fs::remove_file(path);
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
}

#[cfg(unix)]
#[test]
fn unix_session_map_is_0600_and_refuses_group_or_world_readable_files() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let session = tmp.path().join("owner-only.json");
    let mut map = SessionMap::default();
    let _ = mask_tool_result("TFN 123456782", &mut map).unwrap();
    save_session_map(&session, &map).unwrap();
    let mode = fs::metadata(&session).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    fs::set_permissions(&session, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(load_session_map(&session, false).unwrap_err().contains("0600"));
}
