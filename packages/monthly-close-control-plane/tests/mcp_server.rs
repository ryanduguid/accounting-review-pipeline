// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! `close-control mcp`, driven over stdio as an assistant would drive it.

#[path = "support/temp.rs"]
mod temp;

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;

use monthly_close_control_plane::pyjson::{self, Json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
};
use temp::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_close-control");

fn quoted(path: &Path) -> String {
    pyjson::canonical(&Json::str(path.to_str().unwrap()))
}

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")
}

/// The demo pack and copies of the examples the other tools read.
fn workspace(temp: &TempDir) -> PathBuf {
    let root = temp.join("client");
    let demo = child_profile::command(BIN)
        .arg("demo")
        .arg("--output")
        .arg(&root)
        .output()
        .unwrap();
    assert_eq!(demo.status.code(), Some(2), "{demo:?}");
    for name in [
        "journal_entries.csv",
        "schedules/expenses.json",
        "schedules/prepayments.json",
        "schedules/rollforward.json",
        "classification/transactions.csv",
        "classification/coding-evidence.json",
    ] {
        let target = root.join(name.replace('/', "-"));
        fs::copy(examples().join(name), target).unwrap();
    }
    root
}

#[expect(clippy::too_many_lines, reason = "one request per tool")]
fn calls() -> Vec<(i64, String, String)> {
    let path = |name: &str| quoted(Path::new(name));
    vec![
        (
            1,
            "view".into(),
            format!(r#"{{"pack_dir":{}}}"#, path("pack")),
        ),
        (
            2,
            "queue".into(),
            format!(
                r#"{{"pack_dirs":[{}],"status":"REVIEW","summary":true}}"#,
                path("pack")
            ),
        ),
        (
            3,
            "compare".into(),
            format!(
                r#"{{"previous_pack":{p},"current_pack":{p},"previous_tb":{prior},"current_tb":{current},"responses":{note}}}"#,
                p = path("pack"),
                prior = path("inputs/current_trial_balance.csv"),
                current = path("inputs/current_trial_balance.csv"),
                note = path("inputs/review_note.json"),
            ),
        ),
        (
            4,
            "classify".into(),
            format!(
                r#"{{"original_transactions":{t},"current_transactions":{t},"coding_evidence":{e},"currency":"AUD"}}"#,
                t = path("classification-transactions.csv"),
                e = path("classification-coding-evidence.json"),
            ),
        ),
        (
            5,
            "schedule".into(),
            format!(
                r#"{{"kind":"expenses","input":{}}}"#,
                path("schedules-expenses.json")
            ),
        ),
        (
            6,
            "jet".into(),
            format!(
                r#"{{"transactions":{},"currency":"AUD","period_end":"2026-07-31"}}"#,
                path("journal_entries.csv")
            ),
        ),
        (
            7,
            "schedule".into(),
            format!(
                r#"{{"kind":"prepayments","input":{}}}"#,
                path("schedules-prepayments.json")
            ),
        ),
        (
            8,
            "schedule".into(),
            format!(
                r#"{{"kind":"rollforward","input":{}}}"#,
                path("schedules-rollforward.json")
            ),
        ),
    ]
}

#[test]
#[expect(clippy::too_many_lines, reason = "one session through every tool")]
fn an_assistant_session_reads_packs_and_files_inside_the_allowed_directory() {
    let temp = TempDir::new();
    let root = workspace(&temp);
    let mut child = child_profile::command(BIN)
        .arg("mcp")
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = String::from(
        r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#,
    );
    input += "\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n";
    for (id, tool, arguments) in calls() {
        input += &format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"tools/call\",\"params\":{{\"name\":\"{tool}\",\"arguments\":{arguments}}}}}\n"
        );
    }
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let replies: Vec<Json> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| pyjson::loads(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 9);
    assert!(
        replies[0]
            .at("result")
            .at("protocolVersion")
            .is_str("2025-11-25")
    );
    let text = |index: usize| {
        let result = replies[index].at("result");
        let text = result.at("content").as_array().unwrap()[0]
            .at("text")
            .as_pystr()
            .unwrap()
            .to_utf8()
            .unwrap();
        (text, result.at("isError").as_bool().unwrap())
    };
    let (view, failed) = text(1);
    assert!(!failed, "{view}");
    assert!(view.ends_with("it does not approve a close or change any computed status."));
    let (queue, failed) = text(2);
    assert!(!failed && queue.contains("REVIEW"), "{queue}");
    // compare reads every file, then refuses the demo review note as
    // responses for this comparison.
    let (compare, failed) = text(3);
    assert!(failed, "{compare}");
    assert_eq!(
        compare,
        "Responses must use schema 1 and bind to the current pack JSON digest."
    );
    for index in [4, 5, 6] {
        let (result, failed) = text(index);
        assert!(!failed, "{result}");
        let value = pyjson::loads(&result).unwrap();
        assert!(value.get("status").is_some(), "{result}");
    }
    let (jet, _) = text(6);
    assert!(jet.contains("\"unbalanced_journal\""), "{jet}");
    // The prepayment and roll-forward kinds return what the command prints.
    for (index, kind) in [(7, "prepayments"), (8, "rollforward")] {
        let (result, failed) = text(index);
        assert!(!failed, "{result}");
        let command = child_profile::command(BIN)
            .args(["schedule", "--kind", kind, "--input"])
            .arg(root.join(format!("schedules-{kind}.json")))
            .output()
            .unwrap();
        assert_eq!(
            result.trim_end(),
            // The command writes the platform's line endings.
            String::from_utf8(command.stdout)
                .unwrap()
                .replace(
                    "
", "
"
                )
                .trim_end()
        );
    }
}

#[test]
fn the_server_needs_an_existing_allowed_directory() {
    let output = child_profile::command(BIN).arg("mcp").output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("the following arguments are required: DIRECTORY"),
        "{error}"
    );
    let temp = TempDir::new();
    let output = child_profile::command(BIN)
        .arg("mcp")
        .arg(temp.join("absent"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.starts_with("close-control mcp: directory "),
        "{error}"
    );
    let output = child_profile::command(BIN)
        .arg("mcp")
        .arg(&temp.0)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stdout.is_empty());
    let output = child_profile::command(BIN)
        .arg("mcp")
        .arg(&temp.0)
        .arg(&temp.0)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8(output.stderr).unwrap();
    assert_eq!(
        error.trim_end(),
        "close-control mcp: give exactly one DIRECTORY."
    );
}

#[test]
fn the_bundle_manifest_matches_the_server() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest =
        pyjson::loads(&fs::read_to_string(root.join("mcpb/manifest.json")).unwrap()).unwrap();
    let text = |value: &Json| value.as_pystr().unwrap().to_utf8().unwrap();
    let version = fs::read_to_string(root.join("VERSION")).unwrap();
    assert_eq!(text(manifest.at("version")), version.trim());
    let config = manifest.at("server").at("mcp_config");
    let args: Vec<_> = config
        .at("args")
        .as_array()
        .unwrap()
        .iter()
        .map(text)
        .collect();
    assert_eq!(args, ["mcp", "${user_config.client_folder}"]);
    let listed = monthly_close_control_plane::mcp::handle(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        &monthly_close_control_plane::beneath::Root::open(&root).unwrap(),
    )
    .unwrap();
    let names = |tools: &Json| -> Vec<(String, String)> {
        tools
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| (text(tool.at("name")), text(tool.at("description"))))
            .collect()
    };
    assert_eq!(
        names(manifest.at("tools")),
        names(listed.at("result").at("tools"))
    );
}

#[test]
fn help_names_the_directory() {
    let output = child_profile::command(BIN)
        .args(["mcp", "--help"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let help = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(help, include_str!("../src/help/mcp-help.txt"));
}
