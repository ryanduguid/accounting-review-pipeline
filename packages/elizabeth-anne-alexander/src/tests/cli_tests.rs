// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Port of tests/test_cli.py, run in-process against `cli::main`.

use std::path::{Path, PathBuf};

use super::*;
use crate::persist;
use crate::util::Location;

#[path = "../../../../tests/support/directory_link.rs"]
mod directory_link;

fn evaluate_to(env: &Env, out: &str) -> Run {
    let mut args = EVALUATE.to_vec();
    args.extend(["--out", out]);
    run(env, &args)
}

#[cfg(unix)]
#[test]
fn occupied_validation_staging_preserves_inputs_and_recovers() {
    use std::os::unix::fs::MetadataExt;

    for hard_link in [false, true] {
        let dir = TempDir::new();
        let env = env(dir.path());
        let run_dir = run_evaluation(&env);
        let out = dir.path().join("build/validation.json");
        assert_eq!(validate_to(&env, &run_dir, &out).code, 0);
        let summary = std::fs::read(&out).unwrap();
        let inputs = snapshot(&run_dir);
        let unrelated = dir.path().join("unrelated.txt");
        write(&unrelated, "fabricated unrelated target\n");
        let staging = out.with_file_name(".validation.json.partial");
        if hard_link {
            std::fs::hard_link(&unrelated, &staging).unwrap();
        } else {
            write(&staging, "occupied staging\n");
        }
        let staged_bytes = std::fs::read(&staging).unwrap();
        let identity = std::fs::metadata(&staging).unwrap();
        let result = validate_to(&env, &run_dir, &out);
        assert_eq!(result.code, 2, "{}", result.err);
        assert!(result.out.is_empty());
        assert!(
            result.err.contains("validation output cannot be written"),
            "{}",
            result.err
        );
        assert!(result.err.contains("[Errno 17]"), "{}", result.err);
        assert_eq!(std::fs::read(&out).unwrap(), summary);
        assert_eq!(snapshot(&run_dir), inputs);
        assert_eq!(std::fs::read(&staging).unwrap(), staged_bytes);
        let after = std::fs::metadata(&staging).unwrap();
        assert_eq!((after.dev(), after.ino()), (identity.dev(), identity.ino()));
        assert_eq!(
            std::fs::read_to_string(&unrelated).unwrap(),
            "fabricated unrelated target\n"
        );
        std::fs::remove_file(&staging).unwrap();
        let retry = validate_to(&env, &run_dir, &out);
        assert_eq!(retry.code, 0, "{}", retry.err);
        assert!(retry.out.contains("DECISION_RECORDED"));
        assert!(!staging.exists());
        assert_eq!(snapshot(&run_dir), inputs);
        assert_eq!(
            std::fs::read_to_string(&unrelated).unwrap(),
            "fabricated unrelated target\n"
        );
    }
}

#[cfg(unix)]
#[test]
fn public_commands_resolve_repeated_links_and_refuse_unusable_paths() {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new();
    let env = env(dir.path());
    let run_dir = run_evaluation(&env);
    let inputs = snapshot(&run_dir);
    let build = dir.path().join("build");
    symlink("demo", build.join("link")).unwrap();
    let out = build.join("validation.json");
    let result = validate_to(&env, &build.join("link/../link"), &out);
    assert_eq!(result.code, 0, "{}", result.err);
    assert!(result.out.contains("DECISION_RECORDED"));
    let summary = std::fs::read(&out).unwrap();

    symlink("loop", build.join("loop")).unwrap();
    write(&build.join("occupied"), "not a directory\n");
    for unusable in [build.join("loop"), build.join("occupied")] {
        let result = validate_to(&env, &unusable, &out);
        assert_eq!(result.code, 2, "{}", result.err);
        assert!(result.out.is_empty());
        assert!(
            result.err.contains("reviewer evidence is not a usable path"),
            "{}",
            result.err
        );
        assert_eq!(std::fs::read(&out).unwrap(), summary);
    }
    let blocked_out = build.join("loop/validation.json");
    let result = validate_to(&env, &run_dir, &blocked_out);
    assert_eq!(result.code, 2, "{}", result.err);
    assert!(result.out.is_empty());
    let context = build.join("occupied/child.json");
    let mut args = EVALUATE.to_vec();
    args[2] = context.to_str().unwrap();
    args.extend(["--out", "build/refused-context"]);
    let result = run(&env, &args);
    assert!(!build.join("refused-context").exists());
    assert_eq!(result.code, 2, "{}", result.err);
    assert!(result.err.contains("context is not a usable path"), "{}", result.err);
    assert_eq!(snapshot(&run_dir), inputs);
    assert_eq!(
        std::fs::read_to_string(build.join("occupied")).unwrap(),
        "not a directory\n"
    );
    assert_eq!(std::fs::read(&out).unwrap(), summary);
}

#[test]
fn test_cli_end_to_end_from_any_working_directory() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let first = evaluate_to(&env, "build/demo");
    assert_eq!(first.code, 0, "{}", first.err);
    assert!(dir.path().join("build/demo/receipt.json").is_file());
    let second = run(
        &env,
        &[
            "validate-review",
            "--evidence",
            "build/demo/reviewer-evidence.json",
            "--receipt",
            "build/demo/receipt.json",
            "--decision",
            DECISION,
        ],
    );
    assert_eq!(second.code, 0, "{}", second.err);
    let out = first.out + &second.out;
    assert_eq!(out.matches("elizabeth-anne-alexander:").count(), 2);
    assert!(!out.contains("xero-ai-review-gateway:"));
    assert!(!out.contains("xero-ledger-review-gate:"));
}

#[test]
fn test_cli_blocks_output_outside_cwd_build() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let elsewhere = dir.path().join("elsewhere");
    assert_eq!(evaluate_to(&env, elsewhere.to_str().unwrap()).code, 2);
}

#[test]
fn dangling_output_links_publish_only_beneath_build() {
    for spelling in ["absolute", "canonical-parent", "relative", "long-canonical"] {
        let dir = TempDir::new();
        let env = env(dir.path());
        let build = dir.path().join("build");
        std::fs::create_dir(&build).unwrap();
        let name = if spelling == "long-canonical" {
            // Two valid filename components exceed MAX_PATH even in a short temp root.
            PathBuf::from("fabricated-output-".repeat(9)).join("fabricated-output-".repeat(9))
        } else {
            PathBuf::from("new")
        };
        let target = build.join(&name);
        let alias = dir.path().join("linked");
        let link_target = match spelling {
            "absolute" => target.clone(),
            "canonical-parent" | "long-canonical" => std::fs::canonicalize(&build).unwrap().join(&name),
            "relative" => PathBuf::from("build/new"),
            _ => unreachable!(),
        };
        if !directory_link::create(&link_target, &alias) {
            return;
        }
        assert!(!target.exists());
        let result = evaluate_to(&env, "linked/run");
        assert_eq!(result.code, 0, "{}", result.err);
        assert!(result.err.is_empty());
        assert!(result.out.contains("REVIEW_READY"));
        let mut published: Vec<_> = std::fs::read_dir(target.join("run"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_owned())
            .collect();
        published.sort();
        assert_eq!(
            published,
            ["model-result.json", "receipt.json", "reviewer-evidence.json"]
        );
        assert!(std::fs::symlink_metadata(alias).unwrap().file_type().is_symlink());
    }
}

#[test]
fn dangling_output_links_outside_build_are_refused() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let outside = TempDir::new();
    let target = outside.path().join("new");
    if !directory_link::create(&target, &dir.path().join("linked")) {
        return;
    }
    let result = evaluate_to(&env, "linked/run");
    assert_eq!(result.code, 2, "{}", result.err);
    assert!(result.out.is_empty());
    assert!(result.err.contains("output directory must stay within"));
    assert!(!target.exists());
}

#[test]
fn test_a_finished_run_is_not_failed_by_a_path_the_output_stream_cannot_encode() {
    // The binary writes UTF-8, as Python 3.15 does by default, so no character
    // in a path can fail the write that reports a finished run.
    let dir = TempDir::new();
    let workdir = dir.path().join("demo-\u{141}ukasz");
    std::fs::create_dir(&workdir).unwrap();
    let env = env(&workdir);
    let result = evaluate_to(&env, "build/demo");
    assert_eq!(result.code, 0);
    assert!(workdir.join("build/demo/receipt.json").is_file());
    assert!(result.out.contains("\u{141}"));
}

#[test]
fn test_a_run_leaves_the_error_handler_of_every_stream_it_prints_to_alone() {
    // The Rust entry point takes its streams as arguments and never reconfigures
    // them; what remains to check is that both outcomes report through them.
    let dir = TempDir::new();
    let workdir = dir.path().join("demo-\u{141}ukasz");
    std::fs::create_dir(&workdir).unwrap();
    let env = env(&workdir);
    assert_eq!(evaluate_to(&env, "build/demo").code, 0);
    let blocked = evaluate_to(&env, dir.path().join("elsewhere").to_str().unwrap());
    assert_eq!(blocked.code, 2);
    assert!(blocked.err.contains("blocked"));
}

#[test]
fn test_an_output_path_occupied_by_a_file_is_blocked_not_a_traceback() {
    let dir = TempDir::new();
    write(&dir.path().join("build/occupied"), "not a directory\n");
    assert_eq!(evaluate_to(&env(dir.path()), "build/occupied").code, 2);
}

#[test]
fn test_a_validation_output_path_that_is_a_directory_is_blocked_not_a_traceback() {
    let dir = TempDir::new();
    let env = env(dir.path());
    assert_eq!(evaluate_to(&env, "build/demo").code, 0);
    std::fs::create_dir(dir.path().join("build/validation.json")).unwrap();
    let result = run(
        &env,
        &[
            "validate-review",
            "--evidence",
            "build/demo/reviewer-evidence.json",
            "--receipt",
            "build/demo/receipt.json",
            "--decision",
            DECISION,
            "--out",
            "build/validation.json",
        ],
    );
    assert_eq!(result.code, 2);
}

fn run_evaluation(env: &Env) -> PathBuf {
    assert_eq!(evaluate_to(env, "build/demo").code, 0);
    let run_dir = env.cwd.join("build/demo");
    write(&run_dir.join("decision.json"), &shipped(DECISION));
    run_dir
}

fn validate_to(env: &Env, run_dir: &Path, out: &Path) -> Run {
    let arg = |name: &str| run_dir.join(name).to_string_lossy().into_owned();
    let (evidence, receipt, decision) = (arg("reviewer-evidence.json"), arg("receipt.json"), arg("decision.json"));
    let out = out.to_string_lossy().into_owned();
    run(
        env,
        &[
            "validate-review",
            "--evidence",
            &evidence,
            "--receipt",
            &receipt,
            "--decision",
            &decision,
            "--out",
            &out,
        ],
    )
}

fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap_or_default(),
            )
        })
        .collect();
    files.sort();
    files
}

#[test]
fn test_a_validation_output_that_names_an_input_is_blocked_and_the_input_survives() {
    for name in [
        "reviewer-evidence.json",
        "receipt.json",
        "decision.json",
        "model-result.json",
    ] {
        let dir = TempDir::new();
        let env = env(dir.path());
        let run_dir = run_evaluation(&env);
        let before = snapshot(&run_dir);
        let result = validate_to(&env, &run_dir, &run_dir.join(name));
        assert_eq!(result.code, 2);
        assert_eq!(snapshot(&run_dir), before);
        assert!(
            result.err.contains("must not overwrite a review input"),
            "{}",
            result.err
        );
        let fresh = dir.path().join("build/validation.json");
        assert_eq!(validate_to(&env, &run_dir, &fresh).code, 0);
        assert!(fresh.is_file());
    }
}

#[test]
fn test_an_existing_summary_is_replaced_when_the_optional_model_result_is_absent() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let run_dir = run_evaluation(&env);
    std::fs::remove_file(run_dir.join("model-result.json")).unwrap();
    let out = dir.path().join("build/validation.json");
    assert_eq!(validate_to(&env, &run_dir, &out).code, 0);
    let first = std::fs::read(&out).unwrap();
    assert_eq!(validate_to(&env, &run_dir, &out).code, 0);
    assert_eq!(std::fs::read(&out).unwrap(), first);
    assert_eq!(validate_to(&env, &run_dir, &run_dir.join("receipt.json")).code, 2);

    // The by-path branch, which Windows takes, replaces an existing summary too.
    let summary = parse(r#"{"status": "DECISION_RECORDED"}"#);
    let protected = [
        Location::Disk(run_dir.join("receipt.json")),
        Location::Disk(run_dir.join("model-result.json")),
    ];
    persist::write_validation_by_path(&summary, &out, &protected).unwrap();
    persist::write_validation_by_path(&summary, &out, &protected).unwrap();
    assert_eq!(
        read_json(&out).at("status").as_pystr().unwrap().to_utf8().unwrap(),
        "DECISION_RECORDED"
    );
}

#[test]
fn test_validate_review_writes_its_output_on_the_platform_it_runs_on() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let run_dir = run_evaluation(&env);
    let out = dir.path().join("build/validation.json");
    assert_eq!(validate_to(&env, &run_dir, &out).code, 0);
    assert!(read_json(&out).get("status").is_some());
    let leftovers: Vec<_> = std::fs::read_dir(out.parent().unwrap())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".partial"))
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn test_the_by_path_staged_write_still_refuses_a_destination_that_aliases_an_input() {
    let dir = TempDir::new();
    let protected = dir.path().join("receipt.json");
    write(&protected, "{}\n");
    let alias = dir.path().join("validation.json");
    if std::fs::hard_link(&protected, &alias).is_err() {
        return; // filesystem without hard links
    }
    let result = persist::write_validation_by_path(
        &parse(r#"{"status": "DECISION_RECORDED"}"#),
        &alias,
        &[Location::Disk(protected.clone())],
    );
    assert_refused(result, "must not overwrite a review input");
    assert_eq!(std::fs::read_to_string(&protected).unwrap(), "{}\n");
    assert!(
        std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().ends_with(".partial"))
    );
}

#[test]
fn test_a_hard_link_to_an_input_is_blocked() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let run_dir = run_evaluation(&env);
    let alias = dir.path().join("build/alias.json");
    if std::fs::hard_link(run_dir.join("decision.json"), &alias).is_err() {
        return; // filesystem without hard links
    }
    let before = std::fs::read(run_dir.join("decision.json")).unwrap();
    assert_eq!(validate_to(&env, &run_dir, &alias).code, 2);
    assert_eq!(std::fs::read(run_dir.join("decision.json")).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn test_a_symbolic_link_to_an_input_is_blocked() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let run_dir = run_evaluation(&env);
    let alias = dir.path().join("build/alias.json");
    std::os::unix::fs::symlink(run_dir.join("receipt.json"), &alias).unwrap();
    let before = std::fs::read(run_dir.join("receipt.json")).unwrap();
    assert_eq!(validate_to(&env, &run_dir, &alias).code, 2);
    assert_eq!(std::fs::read(run_dir.join("receipt.json")).unwrap(), before);
}

#[test]
fn usage_errors_and_help_match_argparse() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let none = run(&env, &[]);
    assert_eq!(none.code, 2);
    assert_eq!(
        none.err,
        "usage: elizabeth-anne-alexander [-h] {evaluate,validate-review} ...\nelizabeth-anne-alexander: error: the following arguments are required: command\n"
    );
    let typo = run(&env, &["evalute"]);
    assert!(typo.err.ends_with(
        "invalid choice: 'evalute', maybe you meant 'evaluate'? (choose from 'evaluate', 'validate-review')\n"
    ));
    let help = run(&env, &["validate-review", "-h"]);
    assert_eq!(help.code, 0);
    assert!(help.out.contains("  --decision DECISION  decision JSON under the bundled samples/ data or the\n                       working directory's build/\n"));
}

#[test]
fn explicit_help_and_ambiguous_options_match_argparse() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let help = run(&env, &["evaluate", "-hx"]);
    assert_eq!(help.code, 0);
    assert_eq!(help.out, run(&env, &["evaluate", "--help"]).out);
    assert!(help.err.is_empty());
    for (args, message) in [
        (
            vec!["evaluate", "--help=x"],
            "argument -h/--help: ignored explicit argument 'x'",
        ),
        (
            vec!["evaluate", "--=x"],
            "ambiguous option: --=x could match --help, --context, --request, --policy, --out",
        ),
    ] {
        let result = run(&env, &args);
        assert_eq!(result.code, 2);
        assert!(result.out.is_empty());
        assert!(result.err.ends_with(&format!("{message}\n")), "{}", result.err);
    }
    let mut args = EVALUATE.to_vec();
    args.extend(["--out", "build/demo", "extra"]);
    let extra = run(&env, &args);
    assert_eq!(extra.code, 2);
    assert!(extra.err.ends_with("unrecognized arguments: extra\n"), "{}", extra.err);
}

#[test]
fn invalid_validation_paths_preserve_the_existing_output_and_remove_staging() {
    let dir = TempDir::new();
    let output = dir.path().join("validation.json");
    write(&output, "fabricated existing bytes\n");
    let before = snapshot(dir.path());
    let invalid = dir.path().join("invalid\0name.json");
    let protected = [Location::Disk(invalid.clone())];
    let summary = parse(r#"{"status":"DECISION_RECORDED"}"#);
    for result in [
        persist::write_validation(&summary, &output, &protected),
        persist::write_validation_by_path(&summary, &output, &protected),
        persist::write_validation(&summary, &invalid, &[]),
        persist::write_validation_by_path(&summary, &invalid, &[]),
    ] {
        assert_refused(result, "validation output");
    }
    assert_eq!(snapshot(dir.path()), before);
}
