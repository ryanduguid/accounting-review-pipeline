//! A standard stream that cannot be written fails as it did for the Python
//! release, measured against it on a closed pipe: buffered output still pending
//! at exit is exit status 120, a report line that cannot be written is an
//! uncaught OSError (exit 1), a diagnostic that cannot be written loses the
//! original code, and argparse ignores its own write errors.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

/// The OSError line Python printed for a write to a pipe with no reader.
const CLOSED_PIPE: &str = if cfg!(windows) {
    "OSError: [Errno 22] Invalid argument"
} else {
    "BrokenPipeError: [Errno 32] Broken pipe"
};

const EVALUATE: &[&str] = &[
    "evaluate",
    "--context",
    "samples/contexts/sample-monthly-variance.context.json",
    "--request",
    "samples/requests/sample-revenue-variance.request.json",
    "--policy",
    "policy/demo-policy-v1.json",
    "--out",
    "build/demo",
];

fn closed_pipe() -> Stdio {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    writer.into()
}

/// Exit code and the text on the stream that still works, run in a fresh directory.
fn run(args: &[&str], broken_stdout: bool, unbuffered: bool) -> (i32, String) {
    static RUN: AtomicU32 = AtomicU32::new(0);
    let cwd = std::env::temp_dir().join(format!(
        "eaa-streams-{}-{}",
        std::process::id(),
        RUN.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&cwd).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_elizabeth-anne-alexander"));
    command.args(args).current_dir(&cwd).env_remove("PYTHONUNBUFFERED");
    if unbuffered {
        command.env("PYTHONUNBUFFERED", "1");
    }
    let output = if broken_stdout {
        command.stdout(closed_pipe()).stderr(Stdio::piped()).output().unwrap()
    } else {
        command.stdout(Stdio::piped()).stderr(closed_pipe()).output().unwrap()
    };
    std::fs::remove_dir_all(&cwd).unwrap();
    let other = if broken_stdout { output.stderr } else { output.stdout };
    (
        output.status.code().unwrap(),
        String::from_utf8(other).unwrap().replace("\r\n", "\n"),
    )
}

#[test]
fn pending_stdout_fails_at_exit_with_status_120() {
    let flush = format!("Exception ignored while flushing sys.stdout:\n{CLOSED_PIPE}\n");
    assert_eq!(run(&["--help"], true, false), (120, flush.clone()));
    assert_eq!(run(EVALUATE, true, false), (120, flush));
}

#[test]
fn unbuffered_stdout_fails_at_the_write() {
    assert_eq!(run(&["--help"], true, true), (0, String::new()));
    let (code, stderr) = run(EVALUATE, true, true);
    assert_eq!(code, 1);
    assert!(stderr.ends_with(&format!("{CLOSED_PIPE}\n")), "{stderr}");
}

#[test]
fn an_unwritable_diagnostic_loses_the_original_code() {
    let blocked = [
        "evaluate",
        "--context",
        "missing.json",
        "--request",
        "r",
        "--policy",
        "p",
        "--out",
        "o",
    ];
    assert_eq!(run(&["--bogus"], false, false), (120, String::new()));
    assert_eq!(run(&["--bogus"], false, true), (2, String::new()));
    assert_eq!(run(&blocked, false, false), (120, String::new()));
    assert_eq!(run(&blocked, false, true), (1, String::new()));
}
