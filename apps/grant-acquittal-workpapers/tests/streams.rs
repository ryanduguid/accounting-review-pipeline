//! A standard stream that cannot be written fails as it did for the Python
//! script, measured against it on a closed pipe: buffered output still pending
//! at exit is exit status 120, a failed print outside the script's handler is
//! an uncaught OSError (exit 1), and argparse ignores its own write errors.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

const EXAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/two-grants");

/// The OSError line Python printed for a write to a pipe with no reader.
const CLOSED_PIPE: &str = if cfg!(windows) {
    "OSError: [Errno 22] Invalid argument"
} else {
    "BrokenPipeError: [Errno 32] Broken pipe"
};

fn closed_pipe() -> Stdio {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    writer.into()
}

/// Exit code and the text on the stream that still works.
fn run(args: &[&str], broken_stdout: bool, unbuffered: bool) -> (i32, String) {
    static RUN: AtomicU32 = AtomicU32::new(0);
    let out = std::env::temp_dir().join(format!(
        "grant-streams-{}-{}",
        std::process::id(),
        RUN.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&out);
    let mut command = Command::new(env!("CARGO_BIN_EXE_grant-workpaper"));
    command.args(args.iter().map(|a| a.replace("OUT", &out.to_string_lossy())));
    command.env_remove("PYTHONUNBUFFERED");
    if unbuffered {
        command.env("PYTHONUNBUFFERED", "1");
    }
    let output = if broken_stdout {
        command.stdout(closed_pipe()).stderr(Stdio::piped()).output().unwrap()
    } else {
        command.stdout(Stdio::piped()).stderr(closed_pipe()).output().unwrap()
    };
    let _ = std::fs::remove_dir_all(&out);
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
    assert_eq!(run(&["--input", EXAMPLE, "--output", "OUT"], true, false), (120, flush));
}

#[test]
fn unbuffered_stdout_fails_at_the_write() {
    // argparse ignores the failed help; the status line's OSError escapes.
    assert_eq!(run(&["--help"], true, true), (0, String::new()));
    let (code, stderr) = run(&["--input", EXAMPLE, "--output", "OUT"], true, true);
    assert_eq!(code, 1);
    assert!(stderr.ends_with(&format!("{CLOSED_PIPE}\n")), "{stderr}");
}

#[test]
fn an_unwritable_diagnostic_loses_the_original_code() {
    // Buffered stderr keeps the failed line pending until exit (120);
    // unbuffered, argparse's ignored error keeps 2 and the handler's raises (1).
    assert_eq!(run(&["--bogus"], false, false), (120, String::new()));
    assert_eq!(run(&["--bogus"], false, true), (2, String::new()));
    assert_eq!(
        run(&["--input", "missing", "--output", "OUT"], false, false),
        (120, String::new())
    );
    assert_eq!(
        run(&["--input", "missing", "--output", "OUT"], false, true),
        (1, String::new())
    );
}
