use monthly_close_control_plane::{Result, cli, pypath};
use std::{ffi::OsString, path::Path};

fn run(output: &Path) -> Result<()> {
    let output = pypath::resolve(output, false).map_err(|_| "Cannot resolve demo output.")?;
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    for (name, source_month, month, day, opening, closing, previous, decisions, expected) in [
        ("june", "june", "06", "30", "0", "100", None, None, 2),
        (
            "july-draft",
            "july",
            "07",
            "31",
            "100",
            "25",
            Some("june"),
            None,
            2,
        ),
        (
            "july-reviewed",
            "july",
            "07",
            "31",
            "100",
            "25",
            Some("june"),
            Some("july"),
            2,
        ),
        (
            "august",
            "august",
            "08",
            "31",
            "25",
            "0",
            Some("july-reviewed"),
            Some("august"),
            0,
        ),
    ] {
        let mut args: Vec<OsString> = [
            "reconcile",
            "--tenant",
            "Demo",
            "--account-id",
            "clearing",
            "--currency",
            "AUD",
            "--opening-balance",
            opening,
            "--closing-balance",
            closing,
        ]
        .map(OsString::from)
        .into();
        args.extend([
            "--transactions".into(),
            examples
                .join(format!("clearing-{source_month}.csv"))
                .into_os_string(),
            "--period-start".into(),
            format!("2026-{month}-01").into(),
            "--period-end".into(),
            format!("2026-{month}-{day}").into(),
            "--output".into(),
            output.join(name).into_os_string(),
        ]);
        if let Some(previous) = previous {
            args.extend([
                "--opening-items".into(),
                output
                    .join(previous)
                    .join("carry-forward.json")
                    .into_os_string(),
            ]);
        }
        if let Some(decisions) = decisions {
            args.extend([
                "--decisions".into(),
                examples
                    .join(format!("clearing-{decisions}-decisions.csv"))
                    .into_os_string(),
            ]);
        }
        let result = cli::main(args);
        if result != expected {
            return Err(format!(
                "{name}: expected exit {expected}, received {result}"
            ));
        }
    }
    monthly_close_control_plane::outln!(
        "Open {} to inspect the outstanding $25.",
        pypath::text(&output.join("july-reviewed/review.html"))
    );
    Ok(())
}
fn main() {
    std::process::exit(cli::command_thread(|| {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        if args == ["--help"] || args == ["-h"] {
            monthly_close_control_plane::outln!("usage: close-control-clearing-demo --output PATH");
            return 0;
        }
        if args.len() != 2 || args[0] != "--output" {
            monthly_close_control_plane::errln!("usage: close-control-clearing-demo --output PATH");
            return 1;
        }
        match run(Path::new(&args[1])) {
            Ok(()) => 0,
            Err(error) => {
                monthly_close_control_plane::errln!("{error}");
                1
            }
        }
    }));
}
#[cfg(test)]
#[path = "../tests/support/temp.rs"]
mod temp;
#[test]
fn four_periods_preserve_the_review_and_carry_forward_contract() {
    let temp = temp::TempDir::new();
    let output = temp.join("clearing");
    run(&output).unwrap();
    assert!(output.join("august/carry-forward.json").is_file());
    let json = std::fs::read_to_string(output.join("august/reconciliation.json")).unwrap();
    assert!(
        monthly_close_control_plane::pyjson::loads(&json)
            .unwrap()
            .at("status")
            .is_str("PASS")
    );
}
