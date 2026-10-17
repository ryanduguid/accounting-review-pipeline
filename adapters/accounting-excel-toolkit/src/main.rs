use accounting_excel_toolkit::{default_vba_dir, run};
use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let mut out = io::stdout().lock();
    let mut err = io::stderr().lock();
    let result = std::env::current_exe()
        .map_err(|e| e.to_string())
        .and_then(|executable| {
            run(
                &arguments,
                &default_vba_dir(&executable),
                &mut out,
                &mut err,
            )
        });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            let _ = write!(
                err,
                "error: {message}{}",
                if cfg!(windows) { "\r\n" } else { "\n" }
            );
            ExitCode::FAILURE
        }
    }
}
