fn main() -> std::process::ExitCode {
    let code = australian_accounting_power_bi::workflow::command_line(
        std::env::args_os().skip(1).collect(),
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    );
    std::process::ExitCode::from(code)
}
