use accounting_excel_toolkit::aged::cli::{Tool, run};
use std::io::Write;
fn main() -> std::process::ExitCode {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let (code, stdout, stderr) = run(Tool::Replay, &args);
    if std::io::stdout()
        .lock()
        .write_all(stdout.as_bytes())
        .is_err()
        || std::io::stderr()
            .lock()
            .write_all(stderr.as_bytes())
            .is_err()
    {
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::from(code)
}
