use xero_trial_balance_export::{offline, stdio};
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 {
        stdio::err("usage: xero-tb-evaluate FIXTURE.csv\n");
        std::process::exit(2);
    }
    match offline::evaluate(std::path::Path::new(&args[0])) {
        Ok(message) => stdio::out(&message),
        Err(error) => {
            stdio::out(&error.stdout);
            stdio::err(&error.stderr);
            std::process::exit(error.code);
        }
    }
}
