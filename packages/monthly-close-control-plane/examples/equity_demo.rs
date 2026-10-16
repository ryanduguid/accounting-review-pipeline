use monthly_close_control_plane::{
    Result, cli,
    engine::{self, ReviewConfig},
    publication,
    pyjson::{self, Json},
    pypath, sha256, viewer,
};
use std::{fs, path::Path};

fn write(path: &Path, text: &str) -> Result<()> {
    let text = if cfg!(windows) {
        text.replace('\n', "\r\n")
    } else {
        text.to_owned()
    };
    fs::write(path, text).map_err(|e| e.to_string())
}
fn run(output: &Path) -> Result<()> {
    let output = publication::require_output_outside_repository(output)?;
    fs::create_dir(&output).map_err(|e| e.to_string())?;
    let mut hashes = Vec::new();
    for (label, when, amount) in [
        ("prior", "2026-08-31", "100000"),
        ("current", "2026-09-30", "129750"),
    ] {
        let path = output.join(format!("{label}.csv"));
        write(
            &path,
            &format!(
                "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n{when},Synthetic,Assets,A,Bank,090,{amount},0,{amount},0\n{when},Synthetic,Equity,E,Capital,300,0,{amount},0,{amount}\n"
            ),
        )?;
        hashes.push(sha256(&fs::read(path).map_err(|e| e.to_string())?));
    }
    let movements = [
        ("contribution", "0", "25000", "Capital contribution"),
        ("withdrawal", "10000", "0", "Owner withdrawal"),
        ("transfer", "0", "15000", "Evidenced profit transfer"),
    ]
    .into_iter()
    .map(|(key, debit, credit, description)| {
        Json::object([
            ("movement_id", Json::str(key)),
            ("date", Json::str("2026-09-30")),
            ("account_id", Json::str("E")),
            ("debit", Json::str(debit)),
            ("credit", Json::str(credit)),
            ("description", Json::str(description)),
            ("evidence_reference", Json::str(&format!("Synthetic {key}"))),
        ])
    })
    .collect();
    let schedule = Json::object([
        ("schema_version", Json::int(1)),
        ("tenant", Json::str("Synthetic")),
        ("opening_date", Json::str("2026-08-31")),
        ("closing_date", Json::str("2026-09-30")),
        ("currency", Json::str("AUD")),
        ("basis", Json::str("ledger_equity")),
        ("prior_source_sha256", Json::str(&hashes[0])),
        ("current_source_sha256", Json::str(&hashes[1])),
        ("equity_account_ids", Json::Array(vec![Json::str("E")])),
        ("complete", Json::Bool(true)),
        ("movements", Json::Array(movements)),
    ]);
    let schedule_path = output.join("schedule.json");
    write(&schedule_path, &(pyjson::pretty_utf8(&schedule)? + "\n"))?;
    let mut config = ReviewConfig::new(output.join("current.csv"), output.join("prior.csv"));
    config.equity_schedule_path = Some(schedule_path);
    config.equity_currency = Some("AUD".into());
    config.equity_currency_evidence = Some("Fabricated report metadata: AUD".into());
    let pack = engine::review_close(&config)?;
    if !pack
        .equity_reconciliation
        .as_ref()
        .unwrap()
        .at("total")
        .at("unexplained")
        .is_str("-250")
    {
        return Err("Equity example differs from its fabricated -250 expectation.".into());
    }
    publication::write_review_pack(&pack, &output.join("pack"))?;
    viewer::verify_pack(&output.join("pack"))?;
    monthly_close_control_plane::outln!(
        "Equity example verified: -250 unexplained. Pack: {}",
        pypath::text(&output.join("pack"))
    );
    Ok(())
}
fn main() {
    std::process::exit(cli::command_thread(|| {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        if args == ["--help"] || args == ["-h"] {
            monthly_close_control_plane::outln!("usage: close-control-equity-demo --output PATH");
            return 0;
        }
        if args.len() != 2 || args[0] != "--output" {
            monthly_close_control_plane::errln!("usage: close-control-equity-demo --output PATH");
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
fn equity_example_preserves_the_unexplained_amount_and_verified_pack() {
    let temp = temp::TempDir::new();
    let output = temp.join("equity");
    run(&output).unwrap();
    let pack = viewer::verify_pack(&output.join("pack")).unwrap();
    assert!(
        pack.document
            .at("equity_reconciliation")
            .at("total")
            .at("unexplained")
            .is_str("-250")
    );
}
