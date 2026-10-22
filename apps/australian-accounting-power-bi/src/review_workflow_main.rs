fn main() -> std::process::ExitCode {
    // argv is the user's own command line; it grants nothing.
    // nosemgrep: rust.lang.security.args-os.args-os
    let arguments = std::env::args_os().skip(1).collect();
    let code = australian_accounting_power_bi::workflow::command_line(
        arguments,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    );
    std::process::ExitCode::from(code)
}
